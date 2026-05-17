use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use wundler_abs::security::{ResolvedSecurity, SecurityConfig};
use wundler_abs::server::{build_router, AbsConfig};
use wundler_abs::state::AppState;
use wundler_abs::telemetry::TelemetryLogger;
use wundler_graph::ChunkManifest;

/// Verify that `build_router` runs to completion without panicking.
///
/// This test does **not** start an HTTP listener; it only exercises the
/// router-construction code path so that wiring errors (missing state,
/// bad route configuration, etc.) surface at test time rather than at
/// server startup.
#[test]
fn router_builds_without_panic() {
    let manifest = ChunkManifest {
        build_id: "smoke-build-id".to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    };

    let archive_tmp = Box::leak(Box::new(tempfile::TempDir::new().expect("archive tempdir")));
    let archive = wundler_abs::archive::ManifestArchive::open(archive_tmp.path(), 10)
        .expect("open archive");
    let app = AppState {
        manifest: Arc::new(RwLock::new(Arc::new(manifest))),
        archive: Arc::new(archive),
        reload_lock: Arc::new(tokio::sync::Mutex::new(())),
        signature: Arc::new(RwLock::new(None)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    };

    let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
    let telemetry = TelemetryLogger::new(tmp.path()).expect("failed to create TelemetryLogger");

    // Should not panic.
    let _router = build_router(
        app,
        telemetry,
        Arc::new(ResolvedSecurity::from_config(&SecurityConfig::default()).unwrap()),
    );
}

/// Verify that `AbsConfig::default()` produces the expected port and TTL.
#[test]
fn abs_config_has_sensible_defaults() {
    let config = AbsConfig::default();
    assert_eq!(config.port, 8080, "default port should be 8080");
    assert_eq!(config.ttl_seconds, 300, "default TTL should be 300 seconds");
}
