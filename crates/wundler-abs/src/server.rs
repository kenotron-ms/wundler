//! Axum HTTP server for the Asset Bundling Server (ABS).
//!
//! Exposes three routes:
//! * `POST /manifest`  — compute and return a delta manifest
//! * `GET  /health`    — return server liveness + current build ID
//! * `GET  /sw.js`     — serve the embedded Service Worker script

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::{
    extract::{ConnectInfo, FromRequestParts, State},
    http::{header, request::Parts, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::warn;
use wundler_graph::ChunkManifest;

use crate::manifest::compute_delta;
use crate::security::auth::require_bearer;
use crate::security::cors::build_cors;
use crate::security::{ResolvedSecurity, SecurityConfig};
use crate::state::AppState;
use crate::telemetry::TelemetryLogger;
use crate::types::{ManifestRequest, ManifestResponse, TelemetryEvent};

// ---------------------------------------------------------------------------
// Embedded assets
// ---------------------------------------------------------------------------

/// Minimal Service Worker script, embedded at compile time.
///
/// The content of this file is replaced by Task 13; for now it is a
/// placeholder that only calls `skipWaiting()` on install so that the SW
/// takes control immediately.
const SERVICE_WORKER_JS: &str = include_str!("../assets/sw.js");

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Server configuration for a wundler-abs instance.
///
/// All fields have sensible defaults via [`Default`]; callers typically
/// override only the fields that differ from the defaults.
#[derive(Debug, Clone)]
pub struct AbsConfig {
    /// Path to the `ChunkManifest` JSON file on disk.
    pub manifest_path: PathBuf,

    /// Base URL of the CDN that serves chunk assets.
    pub cdn_base_url: String,

    /// Path to the append-only JSONL telemetry log.
    pub telemetry_log: PathBuf,

    /// Path to an Ed25519 signing key in PEM format, or `None` to disable
    /// request signing.
    pub signing_key_pem: Option<PathBuf>,

    /// TCP port the server listens on.
    pub port: u16,

    /// How long clients should cache a manifest response, in seconds.
    pub ttl_seconds: u64,

    /// Directory where the manifest archive stores versioned JSON files.
    pub archive_dir: PathBuf,

    /// Maximum number of manifest versions to retain in the archive.
    pub archive_retention: usize,

    /// Bearer-token authentication configuration.
    pub security: SecurityConfig,
}

impl Default for AbsConfig {
    fn default() -> Self {
        Self {
            manifest_path: PathBuf::from("dist/manifest.json"),
            cdn_base_url: "https://cdn.example.com".to_string(),
            telemetry_log: PathBuf::from("/tmp/wundler-telemetry.jsonl"),
            signing_key_pem: None,
            port: 8080,
            ttl_seconds: 300,
            archive_dir: PathBuf::from("dist/.archive"),
            archive_retention: 10,
            security: SecurityConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared router state
// ---------------------------------------------------------------------------

/// State threaded through every Axum handler.
///
/// Cheap to `Clone` — both fields are internally reference-counted.
#[derive(Clone)]
struct RouterState {
    app: AppState,
    telemetry: TelemetryLogger,
}

// ---------------------------------------------------------------------------
// Optional ConnectInfo extractor
// ---------------------------------------------------------------------------

/// Optional peer-address extractor.
///
/// Reads `ConnectInfo<SocketAddr>` from the request extension map and returns
/// `Some(addr)` when present (real TCP connection) or `None` when absent
/// (e.g. `axum_test::TestServer` which has no underlying socket).
///
/// This lets the same handler enforce loopback in production while remaining
/// fully testable without a live TCP socket.
struct MaybeConnectAddr(Option<SocketAddr>);

impl<S> FromRequestParts<S> for MaybeConnectAddr
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let addr = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0);
        Ok(MaybeConnectAddr(addr))
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response body for `GET /health`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Always `"ok"` when the handler responds.
    pub status: &'static str,

    /// The build ID of the currently loaded manifest.
    pub build_id: String,
}

/// Request body for `POST /reload`.
#[derive(Debug, Deserialize)]
struct ReloadRequest {
    /// Absolute path to the `ChunkManifest` JSON file on disk.
    manifest_path: String,
}

/// Success response body for `POST /reload`.
#[derive(Debug, Serialize)]
struct ReloadResponse {
    /// The `build_id` of the newly loaded manifest.
    build_id: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur while handling a manifest request.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// An unexpected internal error prevented the request from being served.
    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ManifestError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}

// ---------------------------------------------------------------------------
// Router construction
// ---------------------------------------------------------------------------

/// Build and return the Axum [`Router`] without starting a listener.
///
/// Separating router construction from server startup makes the router
/// independently testable without needing a live TCP socket.
///
/// The `security` parameter controls bearer-token authentication. When
/// `security.is_enabled()` is `false` (the default when no `[security]`
/// section is present in `wundler.toml`) the middleware is a transparent
/// pass-through and existing behaviour is unchanged.
pub fn build_router(
    app: AppState,
    telemetry: TelemetryLogger,
    security: Arc<ResolvedSecurity>,
) -> Router {
    let state = RouterState { app, telemetry };
    let cors = build_cors(&security);

    // POST /manifest route, optionally rate-limited per source IP.
    // `.route_layer()` keeps the limiter scoped to this route only; /health
    // and /sw.js are never affected.
    let manifest_route = match security.rate_limiter.clone() {
        None => Router::new().route("/manifest", post(post_manifest)),
        Some(limiter) => Router::new()
            .route("/manifest", post(post_manifest))
            .route_layer(axum::middleware::from_fn_with_state(
                limiter,
                crate::security::ratelimit::rate_limit_mw,
            )),
    };

    Router::new()
        .merge(manifest_route)
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .route("/reload", post(post_reload))
        // Inner: bearer-token authentication.
        .layer(middleware::from_fn_with_state(security, require_bearer))
        // Outer: CORS — applied last so it wraps the auth layer.
        .layer(cors)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Server entrypoint
// ---------------------------------------------------------------------------

/// Load configuration, build the router, and start serving.
///
/// Binds to `0.0.0.0:{config.port}`, logs the listening address, then drives
/// the Axum server to completion (or until the process is signalled).
pub async fn run(config: AbsConfig) -> Result<()> {
    let archive = crate::archive::ManifestArchive::open(
        &config.archive_dir,
        config.archive_retention,
    )
    .context("failed to open manifest archive")?;

    let app = AppState::load_from_disk(
        &config.manifest_path,
        config.cdn_base_url.clone(),
        config.ttl_seconds,
        archive,
    )
    .await?;

    let telemetry = TelemetryLogger::new(&config.telemetry_log)?;
    let security = ResolvedSecurity::from_config(&config.security)
        .context("failed to initialise security config")?;
    let router = build_router(app, telemetry, Arc::new(security));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("wundler-abs listening on {}", local_addr);

    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /manifest` — compute the delta manifest for a client request.
///
/// Reads the current manifest under a shared lock, computes which chunks the
/// client still needs to fetch, overrides `ttl` from the server config, and
/// logs a telemetry event (log failures are only warned, never propagated).
async fn post_manifest(
    State(state): State<RouterState>,
    Json(req): Json<ManifestRequest>,
) -> Result<Json<ManifestResponse>, ManifestError> {
    let manifest = state.app.snapshot().await;

    let mut resp = compute_delta(&manifest, &req, &state.app.cdn_base_url);

    // Override TTL with the server-configured value.
    resp.ttl = state.app.ttl_seconds;

    // Determine which chunk IDs will be served for telemetry purposes.
    let chunks_served: Vec<String> = manifest
        .entry_chunks
        .get(&req.entry_point)
        .cloned()
        .unwrap_or_default();

    let session_id = req
        .build_id
        .clone()
        .unwrap_or_else(|| "anonymous".to_string());

    let event = TelemetryEvent {
        session_id,
        entry_point: req.entry_point.clone(),
        chunks_served,
        client_had: req.cached_hashes.clone(),
        timestamp_ms: now_ms(),
    };

    if let Err(e) = state.telemetry.log(&event) {
        warn!("telemetry log failed: {e}");
    }

    Ok(Json(resp))
}

/// `GET /health` — return a liveness check with the current build ID.
async fn get_health(State(state): State<RouterState>) -> Json<HealthResponse> {
    let manifest = state.app.snapshot().await;
    Json(HealthResponse {
        status: "ok",
        build_id: manifest.build_id.clone(),
    })
}

/// `GET /sw.js` — serve the embedded Service Worker script.
///
/// Returns the script with:
/// * `Content-Type: application/javascript; charset=utf-8`
/// * `Cache-Control: public, max-age=0, must-revalidate`
async fn get_service_worker() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (
                header::CACHE_CONTROL,
                "public, max-age=0, must-revalidate",
            ),
        ],
        SERVICE_WORKER_JS,
    )
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Return the current wall-clock time as milliseconds since the Unix epoch.
///
/// Falls back to `0` if the system clock is before the epoch (which should
/// not happen on any supported platform).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// `POST /reload` — hot-swap the in-memory manifest from a file on disk.
///
/// Operator-only endpoint. When served via a real TCP listener started with
/// [`axum::serve`] + `into_make_service_with_connect_info::<SocketAddr>()`,
/// requests from non-loopback addresses are rejected with `403 Forbidden`.
///
/// When no `ConnectInfo` is present (e.g. `axum_test::TestServer`), the
/// loopback check is skipped — this is intentional and correct for
/// unit-level HTTP testing.
///
/// Body: `{ "manifest_path": "/absolute/path/to/manifest.json" }`
/// Success: `200 OK` `{ "build_id": "<new_build_id>" }`
/// Errors: `422 Unprocessable Entity` `{ "error": "..." }` on file/parse failure
/// `403 Forbidden` if request originates from a non-loopback address
async fn post_reload(
    MaybeConnectAddr(addr): MaybeConnectAddr,
    State(state): State<RouterState>,
    Json(body): Json<ReloadRequest>,
) -> impl IntoResponse {
    // Enforce loopback when ConnectInfo is present (production TCP mode).
    // Skipped in axum_test (no real TCP socket).
    if let Some(addr) = addr {
        if !addr.ip().is_loopback() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "reload is only permitted from loopback addresses"
                })),
            )
                .into_response();
        }
    }

    // Read manifest bytes from disk.
    let json_bytes = match tokio::fs::read(&body.manifest_path).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!("failed to read manifest file: {e}")
                })),
            )
                .into_response();
        }
    };

    // Parse the manifest JSON.
    let new_manifest: ChunkManifest = match serde_json::from_slice(&json_bytes) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!("invalid manifest JSON: {e}")
                })),
            )
                .into_response();
        }
    };

    // Atomically swap the manifest (temporary direct-write; Task 4 will use swap_to).
    let build_id = new_manifest.build_id.clone();
    {
        let mut guard = state.app.manifest.write().await;
        *guard = Arc::new(new_manifest);
    }

    (StatusCode::OK, Json(ReloadResponse { build_id })).into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
        middleware,
    };
    use tokio::sync::RwLock;
    use tower::ServiceExt;
    use wundler_graph::ChunkManifest;

    use crate::security::ratelimit::build_limiter;
    use crate::security::ResolvedSecurity;
    use crate::state::AppState;
    use crate::telemetry::TelemetryLogger;

    /// Build a minimal `AppState` + `TelemetryLogger` pair suitable for unit tests.
    ///
    /// Uses an in-memory `ChunkManifest` (no disk I/O) and a temp file for the
    /// telemetry log.  The `NamedTempFile` drops at the end of this function, but
    /// the open `File` descriptor inside `TelemetryLogger` remains valid (Unix
    /// unlink semantics).  The archive `TempDir` is intentionally leaked so the
    /// directory outlives the test function.
    fn test_state() -> (AppState, TelemetryLogger) {
        let manifest = ChunkManifest {
            build_id: "test-build".to_string(),
            chunks: vec![],
            entry_chunks: HashMap::new(),
            module_index: HashMap::new(),
        };

        // Leak the TempDir so the archive directory stays alive for the test process.
        let archive_tmp = Box::leak(Box::new(tempfile::TempDir::new().expect("archive tempdir")));
        let archive = crate::archive::ManifestArchive::open(archive_tmp.path(), 10)
            .expect("open archive");

        let app = AppState {
            manifest: Arc::new(RwLock::new(Arc::new(manifest))),
            archive: Arc::new(archive),
            reload_lock: Arc::new(tokio::sync::Mutex::new(())),
            cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
            ttl_seconds: 60,
        };

        let tmp = tempfile::NamedTempFile::new().expect("tmp file for telemetry");
        let telemetry = TelemetryLogger::new(tmp.path()).expect("telemetry logger");
        // `tmp` drops here; the unlinked path is fine — the open FD inside
        // TelemetryLogger keeps the inode alive for the duration of the test.
        (app, telemetry)
    }

    /// Wrap `build_router` with an outermost middleware that injects a synthetic
    /// `ConnectInfo<SocketAddr>` so the rate-limiter sees a real IP.
    fn router_with_ip(
        app: AppState,
        telemetry: TelemetryLogger,
        security: Arc<ResolvedSecurity>,
        ip: IpAddr,
    ) -> axum::Router {
        let inner = super::build_router(app, telemetry, security);
        inner.layer(middleware::from_fn(
            move |mut req: Request<Body>, next: middleware::Next| async move {
                let ci: ConnectInfo<SocketAddr> = ConnectInfo(SocketAddr::new(ip, 49_152));
                req.extensions_mut().insert(ci);
                next.run(req).await
            },
        ))
    }

    // P1.3 — rate limiter integration tests

    /// Burst-2 limiter: first two POSTs pass, third returns 429 with JSON body.
    #[tokio::test]
    async fn manifest_post_is_rate_limited() {
        let limiter = build_limiter(NonZeroU32::new(2).unwrap(), NonZeroU32::new(2).unwrap());
        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: Some(limiter),
        });
        let (app, telemetry) = test_state();
        let router = router_with_ip(
            app,
            telemetry,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );

        // First two POSTs fit in the burst.
        for i in 0..2 {
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/manifest")
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "request #{i} should not be rate limited"
            );
        }

        // Third must be 429.
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/manifest")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"], "rate limit exceeded");
    }

    /// A tight rate limit on /manifest must never bleed over to /health.
    #[tokio::test]
    async fn health_is_not_rate_limited() {
        let limiter = build_limiter(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap());
        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: Some(limiter),
        });
        let (app, telemetry) = test_state();
        let router = router_with_ip(
            app,
            telemetry,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );

        for i in 0..20 {
            let resp = router
                .clone()
                .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "/health request #{i} must not be rate limited"
            );
        }
    }

    /// When `ResolvedSecurity.rate_limiter` is `None`, no request should be 429.
    #[tokio::test]
    async fn default_config_has_no_rate_limit() {
        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let router = router_with_ip(
            app,
            telemetry,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );

        for _ in 0..50 {
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/manifest")
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "no rate limit must be applied"
            );
        }
    }
}
