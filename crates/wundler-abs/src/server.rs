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

    Router::new()
        .route("/manifest", post(post_manifest))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .route("/reload", post(post_reload))
        .layer(middleware::from_fn_with_state(security, require_bearer))
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
    let app = AppState::load_from_disk(
        &config.manifest_path,
        config.cdn_base_url.clone(),
        config.ttl_seconds,
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
    let manifest_guard = state.app.manifest.read().await;

    let mut resp = compute_delta(&manifest_guard, &req, &state.app.cdn_base_url);

    // Override TTL with the server-configured value.
    resp.ttl = state.app.ttl_seconds;

    // Determine which chunk IDs will be served for telemetry purposes.
    let chunks_served: Vec<String> = manifest_guard
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
    let manifest_guard = state.app.manifest.read().await;
    Json(HealthResponse {
        status: "ok",
        build_id: manifest_guard.build_id.clone(),
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

    // Atomically swap the manifest.
    let build_id = state.app.reload_manifest(new_manifest).await;

    (StatusCode::OK, Json(ReloadResponse { build_id })).into_response()
}
