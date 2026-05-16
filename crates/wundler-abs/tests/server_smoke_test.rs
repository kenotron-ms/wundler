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

    let app = AppState {
        manifest: Arc::new(RwLock::new(manifest)),
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
