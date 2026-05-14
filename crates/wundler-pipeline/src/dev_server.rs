//! Dev server: Axum-based static file server with on-demand SWC TS→JS transform.
//!
//! `DevServer` serves source modules as native ESM, transforming `.ts`/`.tsx`
//! files on every request via SWC (type annotations stripped, no bundling).
//! It also provides an SSE-based HMR endpoint at `/__wundler__/hmr` and
//! serves the HMR client script at `/__wundler__/hmr-client.js`.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use notify::{EventKind, RecursiveMode, Watcher};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

// ---------------------------------------------------------------------------
// HMR client JavaScript (served at /__wundler__/hmr-client.js)
// ---------------------------------------------------------------------------

const HMR_CLIENT_JS: &str = r#"(function() {
  if (typeof EventSource === 'undefined') return;
  const es = new EventSource('/__wundler__/hmr');
  es.addEventListener('change', (ev) => { console.log('[wundler] change:', ev.data); location.reload(); });
  es.onerror = () => {};
})();"#;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A lightweight dev server that serves source files as native ESM,
/// transforming TypeScript on demand.
#[derive(Clone)]
pub struct DevServer {
    /// Root directory to serve files from.
    pub root: PathBuf,
    /// Port to bind on. Pass `0` to let the OS pick a free port.
    pub port: u16,
}

// ---------------------------------------------------------------------------
// Internal state shared across Axum handlers
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    root: Arc<PathBuf>,
    hmr_tx: broadcast::Sender<String>,
}

// ---------------------------------------------------------------------------
// DevServer impl
// ---------------------------------------------------------------------------

impl DevServer {
    /// Start the server and block until Ctrl-C is received.
    pub async fn start(&self) -> Result<()> {
        let (addr, tx) = self.start_for_test().await?;
        println!("DevServer listening on http://{addr}");
        tokio::signal::ctrl_c().await?;
        let _ = tx.send(());
        Ok(())
    }

    /// Start the server in the background and return the bound address together
    /// with a shutdown sender. Dropping or sending on `tx` stops the server.
    ///
    /// Uses port `0` to let the OS pick a free port when `self.port == 0`.
    pub async fn start_for_test(&self) -> Result<(SocketAddr, oneshot::Sender<()>)> {
        let (hmr_tx, _hmr_rx) = broadcast::channel::<String>(64);

        let state = AppState {
            root: Arc::new(self.root.clone()),
            hmr_tx: hmr_tx.clone(),
        };

        // Spawn a std::thread for the notify file watcher so we don't block
        // the async runtime.  On Modify/Create/Remove events, the relative
        // path is broadcast through the HMR channel.
        let watch_root = self.root.clone();
        let watcher_tx = hmr_tx.clone();
        std::thread::spawn(move || -> anyhow::Result<()> {
            let (tx, rx) = std::sync::mpsc::channel::<notify::Event>();
            let mut watcher = notify::recommended_watcher(move |res| {
                if let Ok(ev) = res {
                    let _ = tx.send(ev);
                }
            })?;
            watcher.watch(&watch_root, RecursiveMode::Recursive)?;
            for ev in rx {
                if matches!(
                    ev.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    for p in &ev.paths {
                        let rel = p
                            .strip_prefix(&watch_root)
                            .unwrap_or(p)
                            .display()
                            .to_string();
                        let _ = watcher_tx.send(rel);
                    }
                }
            }
            Ok(())
        });

        let app = Router::new()
            .route("/__wundler__/hmr-client.js", get(hmr_client))
            .route("/__wundler__/hmr", get(hmr_sse))
            .route("/{*path}", get(serve_module))
            .with_state(state);

        let listener =
            TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], self.port))).await?;
        let addr = listener.local_addr()?;

        let (tx, rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await;
        });

        Ok((addr, tx))
    }
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// Serve the HMR client JavaScript.
async fn hmr_client() -> Response {
    (
        StatusCode::OK,
        [("Content-Type", "application/javascript; charset=utf-8")],
        HMR_CLIENT_JS,
    )
        .into_response()
}

/// SSE endpoint — broadcasts file-change events to connected clients.
async fn hmr_sse(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.hmr_tx.subscribe();
    let stream = BroadcastStream::new(rx).map(|item| {
        let path = item.unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("change").data(path))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Serve a source file, transforming TypeScript on demand.
async fn serve_module(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    let fs_path = state.root.join(&path);

    if !fs_path.exists() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let source = match tokio::fs::read_to_string(&fs_path).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read error: {e}"),
            )
                .into_response()
        }
    };

    let transformed = match transform_on_demand(&path, &source) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("transform error: {e}"),
            )
                .into_response()
        }
    };

    (
        StatusCode::OK,
        [("Content-Type", "application/javascript; charset=utf-8")],
        transformed,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Transform helper
// ---------------------------------------------------------------------------

/// If `name` ends with `.ts` or `.tsx`, parse it via SWC, apply the TypeScript
/// strip transform (removing all type annotations), and emit clean JavaScript.
/// Otherwise return `source` unchanged.
fn transform_on_demand(name: &str, source: &str) -> Result<String> {
    if name.ends_with(".ts") || name.ends_with(".tsx") {
        wundler_transform::swc_util::transform_ts_to_js(name, source)
    } else {
        Ok(source.to_string())
    }
}
