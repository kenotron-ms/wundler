//! Integration tests for `POST /select` — operator-driven rollback.

use std::collections::HashMap;
use std::sync::Arc;

use axum_test::TestServer;
use tempfile::TempDir;
use tokio::sync::{Mutex, RwLock};
use wundler_abs::archive::ManifestArchive;
use wundler_abs::security::{ResolvedSecurity, SecurityConfig};
use wundler_abs::server::build_router;
use wundler_abs::state::AppState;
use wundler_abs::telemetry::TelemetryLogger;
use wundler_graph::types::ChunkManifest;

fn manifest(build_id: &str) -> ChunkManifest {
    ChunkManifest {
        build_id: build_id.to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    }
}

async fn make_server(build_ids: &[&str]) -> (TestServer, AppState, TempDir) {
    let tmp_dir = TempDir::new().expect("tempdir");
    let archive_dir = tmp_dir.path().join("archive");
    let log_path = tmp_dir.path().join("telemetry.jsonl");

    let archive = ManifestArchive::open(&archive_dir, 10).expect("open archive");
    for id in build_ids {
        archive.install(&manifest(id)).expect("install");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let current = build_ids.last().copied().expect("non-empty");
    archive.set_current(current).expect("set_current");
    let initial = archive.load(current).expect("load current");

    let app = AppState {
        manifest: Arc::new(RwLock::new(Arc::new(initial))),
        archive: Arc::new(archive),
        reload_lock: Arc::new(Mutex::new(())),
        signature: Arc::new(RwLock::new(None)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    };
    let telemetry = TelemetryLogger::new(&log_path).expect("telemetry");
    let security = Arc::new(ResolvedSecurity::from_config(&SecurityConfig::default()).unwrap());
    let router = build_router(app.clone(), telemetry, security, None, Arc::new(wundler_abs::metrics::Metrics::new()));

    (TestServer::new(router), app, tmp_dir)
}

#[tokio::test]
async fn select_rolls_back_to_earlier_build() {
    let (server, _state, _dir) = make_server(&["v1", "v2", "v3"]).await;

    // Precondition: /health reports v3.
    let h = server.get("/health").await;
    assert_eq!(h.json::<serde_json::Value>()["build_id"], "v3");

    let resp = server.post("/select").json(&serde_json::json!({"build_id": "v1"})).await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["previous"], "v3");
    assert_eq!(body["current"], "v1");

    let h2 = server.get("/health").await;
    assert_eq!(h2.json::<serde_json::Value>()["build_id"], "v1");

    let v = server.get("/versions").await;
    let arr = v.json::<serde_json::Value>();
    for entry in arr.as_array().unwrap() {
        let is_current = entry["is_current"].as_bool().unwrap_or(false);
        let id = entry["build_id"].as_str().unwrap();
        assert_eq!(is_current, id == "v1", "only v1 must be current");
    }

    let resp2 = server.post("/select").json(&serde_json::json!({"build_id": "v2"})).await;
    resp2.assert_status_ok();
    assert_eq!(resp2.json::<serde_json::Value>()["previous"], "v1");
    assert_eq!(resp2.json::<serde_json::Value>()["current"], "v2");
}

#[tokio::test]
async fn select_unknown_build_id_returns_404() {
    let (server, _state, _dir) = make_server(&["v1"]).await;

    let resp = server.post("/select").json(&serde_json::json!({"build_id": "ghost"})).await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json();
    assert!(
        body["error"].as_str().unwrap_or("").contains("ghost"),
        "error must name the missing build_id, got: {}", body["error"]
    );

    let h = server.get("/health").await;
    assert_eq!(h.json::<serde_json::Value>()["build_id"], "v1");
}

#[tokio::test]
async fn select_returns_409_when_swap_in_progress() {
    let (server, state, _dir) = make_server(&["v1", "v2"]).await;

    // Hold the reload_lock to simulate an in-flight swap.
    let held = state.reload_lock.clone();
    let guard = held.lock().await;

    let resp = server.post("/select").json(&serde_json::json!({"build_id": "v1"})).await;
    resp.assert_status(axum::http::StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json();
    assert!(
        body["error"].as_str().unwrap_or("").contains("in progress"),
        "error must mention 'in progress', got: {}", body["error"]
    );

    drop(guard);
    let resp2 = server.post("/select").json(&serde_json::json!({"build_id": "v1"})).await;
    resp2.assert_status_ok();
}

#[tokio::test]
async fn manifest_in_flight_during_select_observes_prior_build() {
    let (server, state, _dir) = make_server(&["v1", "v2"]).await;

    let snap_before = state.snapshot().await;
    assert_eq!(snap_before.build_id, "v2");

    let resp = server.post("/select").json(&serde_json::json!({"build_id": "v1"})).await;
    resp.assert_status_ok();

    assert_eq!(snap_before.build_id, "v2", "snapshot taken before swap must still observe prior build");

    let snap_after = state.snapshot().await;
    assert_eq!(snap_after.build_id, "v1");
}
