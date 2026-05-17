//! Integration tests for `POST /reload`.
//!
//! Uses `axum_test::TestServer` (no real TCP socket). Because there is no TCP
//! connection, the loopback check is skipped — this is intentional and correct
//! for unit-level HTTP testing.
//!
//! Acceptance: `cargo test -p wundler-abs --test reload_test`
//! reports `test result: ok. 3 passed; 0 failed`.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;

use axum_test::TestServer;
use tempfile::{NamedTempFile, TempDir};
use tokio::sync::RwLock;
use wundler_abs::security::{ResolvedSecurity, SecurityConfig};
use wundler_abs::server::build_router;
use wundler_abs::state::AppState;
use wundler_abs::telemetry::TelemetryLogger;
use wundler_graph::types::ChunkManifest;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal ChunkManifest JSON fixture for testing.
fn manifest_json(build_id: &str) -> String {
    serde_json::json!({
        "build_id": build_id,
        "chunks": [],
        "entry_chunks": {},
        "module_index": {}
    })
    .to_string()
}

/// Build a `TestServer` loaded with a manifest whose build_id is `"initial-id"`.
/// Returns the server and the `TempDir` so callers can inspect the archive on disk.
async fn make_server() -> (TestServer, TempDir) {
    let tmp_dir = TempDir::new().expect("create temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");
    let archive_path = tmp_dir.path().join("archive");

    let initial = ChunkManifest {
        build_id: "initial-id".to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    };
    let archive = wundler_abs::archive::ManifestArchive::open(&archive_path, 10)
        .expect("open archive");
    let app = AppState {
        manifest: Arc::new(RwLock::new(Arc::new(initial))),
        archive: Arc::new(archive),
        reload_lock: Arc::new(tokio::sync::Mutex::new(())),
        signature: Arc::new(RwLock::new(None)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    };

    let telemetry = TelemetryLogger::new(&log_path).expect("create TelemetryLogger");
    let router = build_router(
        app,
        telemetry,
        Arc::new(ResolvedSecurity::from_config(&SecurityConfig::default()).unwrap()),
    );
    let server = TestServer::new(router);

    (server, tmp_dir)
}

// ---------------------------------------------------------------------------
// Test: happy path — reload loads the new manifest
// ---------------------------------------------------------------------------

/// `POST /reload` with a valid manifest file swaps the manifest and returns
/// `{ previous, current }` in the response body.  The archive on disk must
/// contain the newly installed manifest file.
#[tokio::test]
async fn test_reload_loads_new_manifest() {
    let (server, dir) = make_server().await;

    let mut tmp = NamedTempFile::new().expect("create temp manifest file");
    write!(tmp, "{}", manifest_json("new-build-after-reload")).expect("write manifest");

    let resp = server
        .post("/reload")
        .json(&serde_json::json!({
            "manifest_path": tmp.path().to_str().expect("valid UTF-8 path")
        }))
        .await;

    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["previous"], "initial-id");
    assert_eq!(body["current"], "new-build-after-reload");

    // /health now reflects the swap.
    let health = server.get("/health").await;
    health.assert_status_ok();
    let health_body: serde_json::Value = health.json();
    assert_eq!(health_body["build_id"], "new-build-after-reload");

    // The archive on disk now contains the new build.
    let archive_dir = dir.path().join("archive");
    let entry = archive_dir.join("new-build-after-reload.json");
    assert!(entry.is_file(), "archive must contain installed manifest");
}

// ---------------------------------------------------------------------------
// Test: reject missing file with 422
// ---------------------------------------------------------------------------

/// `POST /reload` with a path to a nonexistent file must return 422 and
/// leave the existing manifest unchanged.
#[tokio::test]
async fn test_reload_rejects_missing_file() {
    let (server, _dir) = make_server().await;

    let resp = server
        .post("/reload")
        .json(&serde_json::json!({
            "manifest_path": "/tmp/this-file-absolutely-does-not-exist-wundler-test.json"
        }))
        .await;

    resp.assert_status(axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value = resp.json();
    assert!(
        body["error"].as_str().unwrap_or("").contains("failed to read"),
        "error message must describe the read failure, got: {}",
        body["error"]
    );

    // Existing manifest must still be in place.
    let health = server.get("/health").await;
    health.assert_status_ok();
    let health_body: serde_json::Value = health.json();
    assert_eq!(
        health_body["build_id"], "initial-id",
        "manifest must not change on failed reload"
    );
}

// ---------------------------------------------------------------------------
// Test: reject invalid JSON with 422
// ---------------------------------------------------------------------------

/// `POST /reload` with a file containing invalid JSON must return 422 and
/// leave the existing manifest unchanged.
#[tokio::test]
async fn test_reload_rejects_invalid_json() {
    let (server, _dir) = make_server().await;

    // Write a file that is not valid JSON.
    let mut tmp = NamedTempFile::new().expect("create temp file");
    tmp.write_all(b"this is { not valid json }").expect("write bad bytes");

    let resp = server
        .post("/reload")
        .json(&serde_json::json!({
            "manifest_path": tmp.path().to_str().expect("valid UTF-8 path")
        }))
        .await;

    resp.assert_status(axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value = resp.json();
    assert!(
        body["error"].as_str().unwrap_or("").contains("invalid manifest JSON"),
        "error message must describe the JSON parse failure, got: {}",
        body["error"]
    );

    // Existing manifest must still be in place.
    let health = server.get("/health").await;
    let health_body: serde_json::Value = health.json();
    assert_eq!(
        health_body["build_id"], "initial-id",
        "manifest must not change on parse failure"
    );
}
