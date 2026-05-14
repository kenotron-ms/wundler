//! Axum HTTP server for the Asset Bundling Server (ABS).
//!
//! Exposes three routes:
//! * `POST /manifest`  — compute and return a delta manifest
//! * `GET  /health`    — return server liveness + current build ID
//! * `GET  /sw.js`     — serve the embedded Service Worker script

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tracing::warn;

use crate::manifest::compute_delta;
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
pub fn build_router(app: AppState, telemetry: TelemetryLogger) -> Router {
    let state = RouterState { app, telemetry };

    Router::new()
        .route("/manifest", post(post_manifest))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
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
    let router = build_router(app, telemetry);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("wundler-abs listening on {}", local_addr);

    axum::serve(listener, router).await?;

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
