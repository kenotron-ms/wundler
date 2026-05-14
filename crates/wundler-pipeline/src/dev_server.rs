//! Dev server: Axum-based static file server with on-demand SWC TS→JS transform.
//!
//! `DevServer` serves source modules as native ESM, transforming `.ts`/`.tsx`
//! files on every request via SWC (type annotations stripped, no bundling).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

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
        let state = AppState {
            root: Arc::new(self.root.clone()),
        };

        let app = Router::new()
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
// Route handler
// ---------------------------------------------------------------------------

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
