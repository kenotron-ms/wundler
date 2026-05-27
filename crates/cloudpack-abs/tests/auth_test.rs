//! Tests for bearer-token authentication — unit tests (SecretToken) and
//! HTTP integration tests (require_bearer middleware end-to-end).

use cloudpack_abs::security::auth::SecretToken;

// ── SecretToken unit tests ────────────────────────────────────────────────

/// Debug output must never contain the raw token bytes.
///
/// This is a hard security requirement: if a `SecretToken` is accidentally
/// included in a `tracing` event or a `{:?}` panic message, the token must
/// not be visible.
#[test]
fn test_secret_token_debug_does_not_leak_value() {
    let token = SecretToken::new(b"super-secret-token-value".to_vec());
    let debug_str = format!("{:?}", token);

    assert!(
        !debug_str.contains("super-secret-token-value"),
        "Debug output must not contain the raw token; got: {debug_str}"
    );
    assert!(
        debug_str.contains("REDACTED"),
        "Debug output should contain 'REDACTED'; got: {debug_str}"
    );
}

/// Display output must never contain the raw token bytes.
#[test]
fn test_secret_token_display_does_not_leak_value() {
    let token = SecretToken::new(b"another-secret-12345".to_vec());
    let display_str = format!("{}", token);

    assert!(
        !display_str.contains("another-secret-12345"),
        "Display output must not contain the raw token; got: {display_str}"
    );
    assert!(
        display_str.contains("REDACTED"),
        "Display output should contain 'REDACTED'; got: {display_str}"
    );
}

/// verify() returns true when the provided string matches the stored token.
#[test]
fn test_verify_correct_token_returns_true() {
    let token = SecretToken::new(b"correct-token".to_vec());
    assert!(
        token.verify("correct-token"),
        "verify() should return true for the correct token"
    );
}

/// verify() returns false when the provided string does not match.
#[test]
fn test_verify_wrong_token_returns_false() {
    let token = SecretToken::new(b"correct-token".to_vec());
    assert!(
        !token.verify("wrong-token"),
        "verify() should return false for an incorrect token"
    );
}

/// verify() returns false for an empty string, even if the token is non-empty.
#[test]
fn test_verify_empty_string_returns_false() {
    let token = SecretToken::new(b"non-empty-token".to_vec());
    assert!(
        !token.verify(""),
        "verify() should return false for an empty provided string"
    );
}

// ── SecurityConfig / ResolvedSecurity unit tests ──────────────────────────

use std::io::Write as _;
use tempfile::NamedTempFile;
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig};

/// When no `bearer_token_file` is set, security is disabled (pass-through).
#[test]
fn test_no_config_security_is_not_enabled() {
    let config = SecurityConfig::default();
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    assert!(
        !security.is_enabled(),
        "security should be disabled when no token file is configured"
    );
}

/// When a token file exists and is non-empty, security is enabled.
#[test]
fn test_token_file_present_security_is_enabled() {
    let mut tmp = NamedTempFile::new().expect("create token file");
    write!(tmp, "my-bearer-token").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    assert!(
        security.is_enabled(),
        "security should be enabled when a token file is configured"
    );
}

/// Token files may have a trailing newline (common from `echo` or editors).
/// The token is trimmed before storage.
#[test]
fn test_token_file_with_trailing_newline_is_trimmed() {
    let mut tmp = NamedTempFile::new().expect("create token file");
    writeln!(tmp, "trimmed-token").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    // The stored token should verify against the trimmed string, not the newline-terminated one.
    assert!(
        security.token.as_ref().unwrap().verify("trimmed-token"),
        "token should be trimmed of trailing whitespace"
    );
    assert!(
        !security.token.as_ref().unwrap().verify("trimmed-token\n"),
        "token should not verify against the un-trimmed version"
    );
}

/// A non-existent token file path produces a `SecurityError::TokenFileRead`.
#[test]
fn test_missing_token_file_returns_error() {
    let config = SecurityConfig {
        bearer_token_file: Some("/nonexistent/path/that/does/not/exist/token.txt".into()),
        ..Default::default()
    };
    let result = ResolvedSecurity::from_config(&config);
    assert!(
        result.is_err(),
        "from_config should fail when the token file does not exist"
    );
    // The error message should reference the path.
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("nonexistent"),
        "error message should mention the path; got: {err_str}"
    );
}

// ── HTTP integration tests ────────────────────────────────────────────────
//
// These tests exercise the require_bearer middleware end-to-end through the
// Axum router stack, without opening a TCP socket.

use std::sync::Arc;
use axum::http::{header, HeaderValue, StatusCode};
use axum_test::TestServer;
use tempfile::TempDir;
use cloudpack_abs::server::build_router;
use cloudpack_abs::state::AppState;
use cloudpack_abs::telemetry::TelemetryLogger;
use cloudpack_abs::types::ManifestRequest;

// ── Test helpers ────────────────────────────────────────────────────────────

/// Minimal manifest JSON fixture for auth tests.
fn auth_test_manifest() -> NamedTempFile {
    let json = serde_json::json!({
        "build_id": "auth-test-build",
        "chunks": [],
        "entry_chunks": {},
        "module_index": {}
    });
    let mut file = NamedTempFile::new().expect("create temp manifest");
    write!(file, "{}", json).expect("write manifest");
    file
}

/// Build a `TestServer` with security **disabled** (no token required).
async fn make_open_server() -> (TestServer, TempDir) {
    let manifest = auth_test_manifest();
    let tmp_dir = TempDir::new().expect("create temp dir");
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
    .expect("load AppState");
    let telemetry = TelemetryLogger::new(&log_path).expect("TelemetryLogger");
    let security = ResolvedSecurity::from_config(&SecurityConfig::default())
        .expect("default security");

    let router = build_router(app, telemetry, Arc::new(security), None, Arc::new(cloudpack_abs::metrics::Metrics::new()));
    std::mem::forget(manifest);
    (TestServer::new(router), tmp_dir)
}

/// Build a `TestServer` with security **enabled** using `token` as the bearer token.
async fn make_secured_server(token: &str) -> (TestServer, TempDir, NamedTempFile) {
    let manifest = auth_test_manifest();
    let tmp_dir = TempDir::new().expect("create temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");
    let archive_path = tmp_dir.path().join("archive");

    let mut token_file = NamedTempFile::new().expect("create token file");
    write!(token_file, "{token}").expect("write token");

    let archive = cloudpack_abs::archive::ManifestArchive::open(&archive_path, 10)
        .expect("open archive");
    let app = AppState::load_from_disk(
        manifest.path(),
        "https://cdn.example.com".to_string(),
        300,
        archive,
    )
    .await
    .expect("load AppState");
    let telemetry = TelemetryLogger::new(&log_path).expect("TelemetryLogger");
    let config = SecurityConfig {
        bearer_token_file: Some(token_file.path().to_path_buf()),
        ..Default::default()
    };
    let security = ResolvedSecurity::from_config(&config).expect("ResolvedSecurity");

    let router = build_router(app, telemetry, Arc::new(security), None, Arc::new(cloudpack_abs::metrics::Metrics::new()));
    std::mem::forget(manifest);
    (TestServer::new(router), tmp_dir, token_file)
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// When no `[security]` config is present, unauthenticated requests to
/// `/manifest` are accepted — exactly today's behaviour.
#[tokio::test]
async fn test_no_security_config_allows_unauthenticated_requests() {
    let (server, _dir) = make_open_server().await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server.post("/manifest").json(&req).await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "unauthenticated request should not receive 401 when security is disabled"
    );
}

/// A correct `Authorization: Bearer <token>` header is accepted.
#[tokio::test]
async fn test_valid_bearer_token_allows_request() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server
        .post("/manifest")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer my-secret-token"),
        )
        .json(&req)
        .await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "correct bearer token should not receive 401"
    );
}

/// A missing `Authorization` header returns 401.
#[tokio::test]
async fn test_missing_authorization_header_returns_401() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server.post("/manifest").json(&req).await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "missing Authorization header should return 401"
    );
    assert_eq!(resp.text(), "Unauthorized");
}

/// A wrong bearer token returns 401 with the same body as a missing header.
#[tokio::test]
async fn test_wrong_bearer_token_returns_401() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server
        .post("/manifest")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-token"),
        )
        .json(&req)
        .await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "wrong bearer token should return 401"
    );
    assert_eq!(resp.text(), "Unauthorized");
}

/// An `Authorization: Basic ...` header (wrong scheme) returns 401.
#[tokio::test]
async fn test_wrong_auth_scheme_returns_401() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server
        .post("/manifest")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        )
        .json(&req)
        .await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "Basic auth scheme should return 401"
    );
}

/// `GET /health` is always accessible, even when security is enabled.
#[tokio::test]
async fn test_health_always_accessible_without_token() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let resp = server.get("/health").await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "/health should be exempt from auth"
    );
    resp.assert_status_ok();
}

/// `GET /sw.js` is always accessible, even when security is enabled.
#[tokio::test]
async fn test_sw_js_always_accessible_without_token() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let resp = server.get("/sw.js").await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "/sw.js should be exempt from auth"
    );
    resp.assert_status_ok();
}

/// The 401 response carries a `WWW-Authenticate: Bearer` header.
#[tokio::test]
async fn test_401_response_carries_www_authenticate_header() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server.post("/manifest").json(&req).await;

    assert_eq!(resp.status_code(), StatusCode::UNAUTHORIZED);
    let www_auth = resp
        .headers()
        .get("www-authenticate")
        .expect("WWW-Authenticate header should be present on 401")
        .to_str()
        .expect("WWW-Authenticate should be valid UTF-8");
    assert_eq!(www_auth, "Bearer", "WWW-Authenticate value should be 'Bearer'");
}
