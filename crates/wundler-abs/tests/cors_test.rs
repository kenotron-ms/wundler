//! HTTP integration tests for the CORS allowlist (P1.2).

use std::sync::Arc;

use axum_test::TestServer;
use wundler_abs::security::{ResolvedSecurity, SecurityConfig};
use wundler_abs::server::build_router;
use wundler_abs::state::AppState;
use wundler_abs::telemetry::TelemetryLogger;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn make_server(allowed_origins: Vec<String>) -> (TestServer, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let manifest_path = tmp.path().join("manifest.json");
    let archive_path = tmp.path().join("archive");
    let manifest_json = serde_json::json!({
        "build_id": "test-build",
        "chunks": [],
        "entry_chunks": {},
        "module_index": {}
    }).to_string();
    std::fs::write(&manifest_path, manifest_json).expect("write manifest");

    let archive = wundler_abs::archive::ManifestArchive::open(&archive_path, 10)
        .expect("open archive");
    let app = AppState::load_from_disk(
        &manifest_path,
        "https://cdn.example.com".to_string(),
        300,
        archive,
    )
    .await
    .expect("load app state");

    let telem_path = tmp.path().join("telemetry.jsonl");
    let telemetry = TelemetryLogger::new(&telem_path).expect("telemetry");

    let security = ResolvedSecurity::from_config(&SecurityConfig {
        bearer_token_file: None,
        allowed_origins,
        ..SecurityConfig::default()
    })
    .expect("resolve security");

    let router = build_router(app, telemetry, Arc::new(security), None);
    (TestServer::new(router), tmp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_allowlist_emits_no_cors_header_on_simple_request() {
    let (server, _tmp) = make_server(vec![]).await;

    let resp = server
        .get("/health")
        .add_header("origin", "https://app.example.com")
        .await;

    resp.assert_status_ok();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "empty allowlist must not emit Access-Control-Allow-Origin"
    );
}

#[tokio::test]
async fn allowlisted_origin_is_reflected_in_response() {
    let (server, _tmp) = make_server(vec!["https://app.example.com".to_string()]).await;

    let resp = server
        .get("/health")
        .add_header("origin", "https://app.example.com")
        .await;

    resp.assert_status_ok();
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("Access-Control-Allow-Origin must be present")
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://app.example.com");
}

#[tokio::test]
async fn non_allowlisted_origin_gets_no_cors_header() {
    let (server, _tmp) = make_server(vec!["https://app.example.com".to_string()]).await;

    let resp = server
        .get("/health")
        .add_header("origin", "https://evil.example.com")
        .await;

    resp.assert_status_ok();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "non-allowlisted origin must NOT receive Access-Control-Allow-Origin"
    );
}

#[tokio::test]
async fn preflight_options_for_allowlisted_origin_returns_cors_headers() {
    let (server, _tmp) = make_server(vec!["https://app.example.com".to_string()]).await;

    let resp = server
        .method(axum::http::Method::OPTIONS, "/manifest")
        .add_header("origin", "https://app.example.com")
        .add_header("access-control-request-method", "POST")
        .add_header("access-control-request-headers", "authorization,content-type")
        .await;

    // tower-http CORS answers preflight OPTIONS with 200
    resp.assert_status_ok();

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("preflight must echo Access-Control-Allow-Origin")
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://app.example.com");

    let methods = resp
        .headers()
        .get("access-control-allow-methods")
        .expect("preflight must include Access-Control-Allow-Methods")
        .to_str()
        .unwrap()
        .to_ascii_uppercase();
    assert!(methods.contains("GET"), "GET must be allowed");
    assert!(methods.contains("POST"), "POST must be allowed");

    let max_age = resp
        .headers()
        .get("access-control-max-age")
        .expect("preflight must include Access-Control-Max-Age")
        .to_str()
        .unwrap();
    assert_eq!(max_age, "300", "max_age must be 300s");

    assert!(
        resp.headers().get("access-control-allow-credentials").is_none(),
        "Access-Control-Allow-Credentials must NEVER be set"
    );
}

#[tokio::test]
async fn cors_layer_is_outer_so_401_responses_still_carry_cors_header() {
    // Bearer auth + CORS both enabled. An unauthenticated request should get
    // 401 from auth but still get Access-Control-Allow-Origin from outer CORS.
    let tmp = tempfile::tempdir().expect("tmpdir");
    let token_path = tmp.path().join("token");
    std::fs::write(&token_path, "s3cret").expect("write token");

    let manifest_path = tmp.path().join("manifest.json");
    let manifest_json = serde_json::json!({
        "build_id": "test-build",
        "chunks": [],
        "entry_chunks": {},
        "module_index": {}
    }).to_string();
    std::fs::write(&manifest_path, manifest_json).expect("write manifest");

    let archive_path = tmp.path().join("archive");
    let archive = wundler_abs::archive::ManifestArchive::open(&archive_path, 10)
        .expect("open archive");
    let app = AppState::load_from_disk(
        &manifest_path,
        "https://cdn.example.com".to_string(),
        300,
        archive,
    )
    .await
    .expect("load app state");

    let telem_path = tmp.path().join("telemetry.jsonl");
    let telemetry = TelemetryLogger::new(&telem_path).expect("telemetry");

    let security = ResolvedSecurity::from_config(&SecurityConfig {
        bearer_token_file: Some(token_path),
        allowed_origins: vec!["https://app.example.com".to_string()],
        ..SecurityConfig::default()
    })
    .expect("resolve security");

    let router = build_router(app, telemetry, Arc::new(security), None);
    let server = TestServer::new(router);

    let resp = server
        .post("/manifest")
        .add_header("origin", "https://app.example.com")
        .json(&serde_json::json!({
            "entry_point": "main",
            "cached_hashes": [],
            "build_id": null,
        }))
        .await;

    assert_eq!(resp.status_code(), axum::http::StatusCode::UNAUTHORIZED);

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("401 response must still carry Access-Control-Allow-Origin (CORS is outer)")
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://app.example.com");
}
