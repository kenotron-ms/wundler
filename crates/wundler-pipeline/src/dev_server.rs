//! Dev server: Axum-based static file server with on-demand SWC TS→JS transform.
//!
//! `DevServer` serves source modules as native ESM, transforming `.ts`/`.tsx`
//! files on every request via SWC (type annotations stripped, no bundling).
//! It also provides an SSE-based HMR endpoint at `/__wundler__/hmr` and
//! serves the HMR client script at `/__wundler__/hmr-client.js`.
//!
//! ## Root `/` handler
//!
//! A request to `/` returns a generated dev-mode `index.html` that:
//! - Provides an [import map] mapping bare specifiers (`react`, `react-dom`,
//!   `react-dom/client`, `react/jsx-runtime`) to the esm.sh CDN.
//! - Loads the HMR client script (`/__wundler__/hmr-client.js`).
//! - Bootstraps the application via `<script type="module" src="./main.tsx">`.
//!
//! ## Extension resolution
//!
//! Browser native ESM resolves relative imports without file extensions as-is:
//! `import App from './App'` → `GET /App`.  Because browsers cannot try
//! alternative extensions, the server does so on their behalf: if the exact
//! path is not found and the path has no extension, the server tries `.tsx`,
//! `.ts`, `.jsx`, `.js` in that order before returning 404.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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
// Dev index HTML (served at /)
//
// Provides an import map for React CDN (esm.sh), injects the HMR client, and
// bootstraps the app from the source entry point `./main.tsx`.
// ---------------------------------------------------------------------------

const DEV_INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Wundler Dev — Taskflow</title>
  <script type="importmap">
  {
    "imports": {
      "react":             "https://esm.sh/react@19.0.0",
      "react-dom":         "https://esm.sh/react-dom@19.0.0",
      "react-dom/client":  "https://esm.sh/react-dom@19.0.0/client",
      "react/jsx-runtime": "https://esm.sh/react@19.0.0/jsx-runtime"
    }
  }
  </script>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: system-ui, -apple-system, sans-serif; background: #f5f5f5; }
    #root { min-height: 100vh; }
  </style>
</head>
<body>
  <div id="root"></div>
  <!-- HMR client: subscribes to /__wundler__/hmr SSE and calls location.reload() on change -->
  <script defer src="/__wundler__/hmr-client.js"></script>
  <!-- App entry point: SWC transforms .tsx on demand -->
  <script type="module" src="./main.tsx"></script>
</body>
</html>"#;

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
            // Serve the generated dev index.html at the root.
            .route("/", get(index_handler))
            .route("/__wundler__/hmr-client.js", get(hmr_client))
            .route("/__wundler__/hmr", get(hmr_sse))
            // Catch-all: serve source modules with on-demand TS transform.
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

/// Serve the generated dev-mode index.html at `/`.
async fn index_handler() -> Response {
    (
        StatusCode::OK,
        [("Content-Type", "text/html; charset=utf-8")],
        DEV_INDEX_HTML,
    )
        .into_response()
}

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
///
/// Extension resolution: if the exact path is not found and the URL path has
/// no file extension, the server tries `.tsx`, `.ts`, `.jsx`, `.js` in order.
/// This lets browsers fetch `import './App'` and receive `App.tsx`.
async fn serve_module(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    let fs_path = match resolve_module_path(&state.root, &path) {
        Some(p) => p,
        None => {
            return (StatusCode::NOT_FOUND, format!("module not found: {path}"))
                .into_response()
        }
    };

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

    // Use the resolved filesystem path for extension detection so that
    // extension-less URL requests (e.g. `/App` → `App.tsx`) are transformed
    // correctly.
    let fs_name = fs_path.to_string_lossy();
    let transformed = match transform_on_demand(&fs_name, &source) {
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
// Extension resolver
// ---------------------------------------------------------------------------

/// Resolve `path` (a URL path component) against `root`, trying the exact
/// filename first, then common JS/TS extensions when the path has none.
///
/// Returns `None` if no matching file is found.
fn resolve_module_path(root: &Path, path: &str) -> Option<PathBuf> {
    let exact = root.join(path);
    if exact.is_file() {
        return Some(exact);
    }

    // Extension-less import — try common JS/TS extensions in priority order.
    if Path::new(path).extension().is_none() {
        for ext in &["tsx", "ts", "jsx", "js"] {
            let candidate = root.join(format!("{path}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
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
