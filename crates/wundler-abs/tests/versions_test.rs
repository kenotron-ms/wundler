//! Integration tests for `GET /versions`.

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

/// Build a TestServer whose archive contains `build_ids` (in order;
/// the LAST id becomes `current`).
async fn make_server(build_ids: &[&str]) -> (TestServer, TempDir) {
    let tmp_dir = TempDir::new().expect("tempdir");
    let archive_dir = tmp_dir.path().join("archive");
    let log_path = tmp_dir.path().join("telemetry.jsonl");

    let archive = ManifestArchive::open(&archive_dir, 10).expect("open archive");
    for id in build_ids {
        archive.install(&manifest(id)).expect("install");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let current = build_ids.last().copied().unwrap_or("none");
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
    let router = build_router(app, telemetry, security, None, Arc::new(wundler_abs::metrics::Metrics::new()));

    (TestServer::new(router), tmp_dir)
}

#[tokio::test]
async fn versions_returns_newest_first() {
    let (server, _dir) = make_server(&["v1", "v2", "v3"]).await;
    let resp = server.get("/versions").await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    let arr = body.as_array().expect("array");
    let ids: Vec<&str> = arr.iter().map(|e| e["build_id"].as_str().expect("build_id")).collect();
    assert_eq!(ids, vec!["v3", "v2", "v1"]);
}

#[tokio::test]
async fn versions_marks_current_entry() {
    let (server, _dir) = make_server(&["v1", "v2", "v3"]).await;
    let resp = server.get("/versions").await;
    let body: serde_json::Value = resp.json();
    let arr = body.as_array().expect("array");
    let current_ids: Vec<&str> = arr.iter()
        .filter(|e| e["is_current"].as_bool().unwrap_or(false))
        .map(|e| e["build_id"].as_str().unwrap())
        .collect();
    assert_eq!(current_ids, vec!["v3"], "exactly one entry must have is_current=true");
}

#[tokio::test]
async fn versions_entries_include_bytes_and_timestamp() {
    let (server, _dir) = make_server(&["only-build"]).await;
    let resp = server.get("/versions").await;
    let body: serde_json::Value = resp.json();
    let entry = &body[0];
    assert_eq!(entry["build_id"], "only-build");
    assert!(entry["bytes"].as_u64().expect("bytes") > 0);
    assert!(entry["installed_at_ms"].as_u64().expect("installed_at_ms") > 0);
    assert_eq!(entry["is_current"], true);
}
