//! End-to-end HTTP integration tests for cloudpack-abs.
//!
//! Exercises the full Axum router stack (`POST /manifest`, `GET /health`,
//! `GET /sw.js`) through [`axum_test::TestServer`] without opening a TCP
//! socket.

use std::io::Write as _;
use std::sync::Arc;

use axum_test::TestServer;
use tempfile::{NamedTempFile, TempDir};
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig};
use cloudpack_abs::server::build_router;
use cloudpack_abs::state::AppState;
use cloudpack_abs::telemetry::TelemetryLogger;
use cloudpack_abs::types::{ManifestRequest, ManifestResponse};
use cloudpack_core::types::ContentHash;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Write the integration-test manifest to a [`NamedTempFile`] and return it.
///
/// Manifest layout:
///
/// - `build_id`: `"INT-TEST-BUILD"`
/// - `"shell"` chunk: 1 module (`shell_mod_hash`), `Initial`, CDN prefix `bbbb1111`
/// - `"channel"` chunk: 1 module (`channel_mod_hash`), `Lazy`, CDN prefix `aaaa2222`
/// - Entry point: `"teams.channel"` → `["shell", "channel"]`
/// - `module_index`: `{ "shell_mod_hash": "shell", "channel_mod_hash": "channel" }`
fn manifest_file() -> NamedTempFile {
    let json = serde_json::json!({
        "build_id": "INT-TEST-BUILD",
        "chunks": [
            {
                "id": "shell",
                "modules": ["shell_mod_hash"],
                "hash": "bbbb1111aabbccddeeff00112233445566778899aabbccddeeff001122334455",
                "load_condition": "Initial",
                "co_request_score": null,
                "median_load_order": null,
                "suggested_merge": null
            },
            {
                "id": "channel",
                "modules": ["channel_mod_hash"],
                "hash": "aaaa2222aabbccddeeff00112233445566778899aabbccddeeff001122334455",
                "load_condition": "Lazy",
                "co_request_score": null,
                "median_load_order": null,
                "suggested_merge": null
            }
        ],
        "entry_chunks": {
            "teams.channel": ["shell", "channel"]
        },
        "module_index": {
            "shell_mod_hash": "shell",
            "channel_mod_hash": "channel"
        }
    });

    let mut file = NamedTempFile::new().expect("failed to create temp manifest file");
    write!(file, "{}", json).expect("failed to write manifest JSON");
    file
}

/// Construct a [`TestServer`] backed by the integration-test manifest.
///
/// Returns `(TestServer, TempDir)`:
/// - The telemetry JSONL log is at `<TempDir>/telemetry.jsonl`.
/// - The manifest [`NamedTempFile`] is leaked via [`std::mem::forget`] so the
///   temporary file survives for the full duration of the test.
async fn make_server() -> (TestServer, TempDir) {
    let manifest = manifest_file();
    let tmp_dir = TempDir::new().expect("failed to create temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");
    let archive_path = tmp_dir.path().join("archive");

    let archive = cloudpack_abs::archive::ManifestArchive::open(&archive_path, 10)
        .expect("open archive");
    let app = AppState::load_from_disk(
        manifest.path(),
        "https://cdn.example.com".to_string(),
        300,
        archive,
    )
    .await
    .expect("failed to load AppState from manifest");

    let telemetry = TelemetryLogger::new(&log_path).expect("failed to create TelemetryLogger");
    let router = build_router(
        app,
        telemetry,
        Arc::new(ResolvedSecurity::from_config(&SecurityConfig::default()).unwrap()),
        None,
        Arc::new(cloudpack_abs::metrics::Metrics::new()),
    );
    let server = TestServer::new(router);

    // Prevent the NamedTempFile destructor from deleting the manifest file
    // before the server is done with it.
    std::mem::forget(manifest);

    (server, tmp_dir)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `GET /health` returns HTTP 200 with `status: "ok"` and the manifest build ID.
#[tokio::test]
async fn health_returns_build_id() {
    let (server, _dir) = make_server().await;

    let resp = server.get("/health").await;
    resp.assert_status_ok();

    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "ok", "expected status 'ok'");
    assert_eq!(
        body["build_id"], "INT-TEST-BUILD",
        "expected build_id 'INT-TEST-BUILD'"
    );
}

/// `POST /manifest` with an empty cache and a matching build ID returns all
/// chunks and the server-configured TTL.
#[tokio::test]
async fn manifest_returns_full_set_on_empty_cache() {
    let (server, _dir) = make_server().await;

    let req = ManifestRequest {
        entry_point: "teams.channel".to_string(),
        cached_hashes: vec![],
        build_id: Some("INT-TEST-BUILD".to_string()),
    };

    let resp = server.post("/manifest").json(&req).await;
    resp.assert_status_ok();

    let body: ManifestResponse = resp.json();
    assert_eq!(
        body.fetch_urls.len(),
        2,
        "expected 2 fetch_urls for empty cache, got: {:?}",
        body.fetch_urls
    );
    assert_eq!(body.ttl, 300, "expected ttl=300");
}

/// `POST /manifest` — client already holds the shell module → only the channel
/// chunk URL is returned, and it contains `"aaaa2222"`.
#[tokio::test]
async fn manifest_filters_cached_chunks() {
    let (server, _dir) = make_server().await;

    let req = ManifestRequest {
        entry_point: "teams.channel".to_string(),
        cached_hashes: vec![ContentHash("shell_mod_hash".to_string())],
        build_id: Some("INT-TEST-BUILD".to_string()),
    };

    let resp = server.post("/manifest").json(&req).await;
    resp.assert_status_ok();

    let body: ManifestResponse = resp.json();
    assert_eq!(
        body.fetch_urls.len(),
        1,
        "expected 1 fetch_url (shell cached), got: {:?}",
        body.fetch_urls
    );
    assert!(
        body.fetch_urls[0].contains("aaaa2222"),
        "expected channel chunk URL to contain 'aaaa2222', got: {}",
        body.fetch_urls[0]
    );
}

/// `POST /manifest` — client has a cached module but supplies a stale build ID
/// → the cache is ignored and all chunks are returned.
#[tokio::test]
async fn manifest_returns_full_set_on_stale_build_id() {
    let (server, _dir) = make_server().await;

    let req = ManifestRequest {
        entry_point: "teams.channel".to_string(),
        cached_hashes: vec![ContentHash("shell_mod_hash".to_string())],
        build_id: Some("OLD-BUILD".to_string()),
    };

    let resp = server.post("/manifest").json(&req).await;
    resp.assert_status_ok();

    let body: ManifestResponse = resp.json();
    assert_eq!(
        body.fetch_urls.len(),
        2,
        "stale build_id should return all chunks, got: {:?}",
        body.fetch_urls
    );
}

/// `GET /sw.js` returns HTTP 200 with a JavaScript content-type and a body
/// that contains the install event listener.
#[tokio::test]
async fn sw_js_is_served_with_javascript_content_type() {
    let (server, _dir) = make_server().await;

    let resp = server.get("/sw.js").await;
    resp.assert_status_ok();

    let content_type = resp
        .headers()
        .get("content-type")
        .expect("content-type header missing")
        .to_str()
        .expect("content-type is not valid UTF-8");

    assert!(
        content_type.contains("javascript"),
        "expected content-type to contain 'javascript', got: {content_type}"
    );

    let body = resp.text();
    assert!(
        body.contains("install"),
        "expected sw.js body to contain 'install', got: {body}"
    );
}

/// Two consecutive `POST /manifest` requests each produce one telemetry log
/// entry → the log file ends up with exactly 2 lines.
#[tokio::test]
async fn telemetry_is_written_on_each_request() {
    let (server, dir) = make_server().await;
    let log_path = dir.path().join("telemetry.jsonl");

    let req = ManifestRequest {
        entry_point: "teams.channel".to_string(),
        cached_hashes: vec![],
        build_id: Some("INT-TEST-BUILD".to_string()),
    };

    server.post("/manifest").json(&req).await;
    server.post("/manifest").json(&req).await;

    let log = std::fs::read_to_string(&log_path).expect("failed to read telemetry log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 telemetry log lines, got {}: {:?}",
        lines.len(),
        lines
    );
}

/// When security is enabled with a bearer token:
/// - Unauthenticated requests to non-exempt paths return 401.
/// - Requests with the correct Bearer token succeed (200).
/// - Exempt paths (`/health`, `/sw.js`) are always accessible without auth.
#[tokio::test]
async fn test_bearer_auth_enforced_when_security_enabled() {
    // Write a known token to a temp file.
    let mut token_file = NamedTempFile::new().expect("create token file");
    write!(token_file, "test-secret-token").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(token_file.path().to_path_buf()),
        ..Default::default()
    };
    let security = Arc::new(ResolvedSecurity::from_config(&config).expect("resolve security"));

    let manifest_f = manifest_file();
    let tmp_dir = TempDir::new().expect("temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");
    let archive_path = tmp_dir.path().join("archive");

    let archive = cloudpack_abs::archive::ManifestArchive::open(&archive_path, 10)
        .expect("open archive");
    let app = AppState::load_from_disk(
        manifest_f.path(),
        "https://cdn.example.com".to_string(),
        300,
        archive,
    )
    .await
    .expect("load app state");

    let telemetry = TelemetryLogger::new(&log_path).expect("create TelemetryLogger");
    let router = build_router(app, telemetry, security, None, Arc::new(cloudpack_abs::metrics::Metrics::new()));
    let server = TestServer::new(router);
    std::mem::forget(manifest_f);

    let req = ManifestRequest {
        entry_point: "teams.channel".to_string(),
        cached_hashes: vec![],
        build_id: Some("INT-TEST-BUILD".to_string()),
    };

    // Without auth: 401 Unauthorized.
    let resp = server.post("/manifest").json(&req).await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    // With the correct Bearer token: 200 OK.
    let resp = server
        .post("/manifest")
        .authorization_bearer("test-secret-token")
        .json(&req)
        .await;
    resp.assert_status_ok();

    // /health is exempt — no auth required.
    let resp = server.get("/health").await;
    resp.assert_status_ok();
}
