//! Application state for the Asset Bundling Server.
//!
//! [`AppState`] wraps a [`ChunkManifest`] in `Arc<RwLock<_>>` to support
//! future hot-reload without restarting the server.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use wundler_graph::ChunkManifest;

/// Shared application state carried by every Axum handler.
///
/// All fields are cheaply `Clone`-able: cloning an `AppState` shares the same
/// underlying data via `Arc`.
#[derive(Clone)]
pub struct AppState {
    /// The chunk manifest, guarded for future hot-reload support.
    pub manifest: Arc<RwLock<ChunkManifest>>,

    /// Base URL of the CDN that serves chunk assets.
    pub cdn_base_url: Arc<String>,

    /// How long clients should cache a manifest response, in seconds.
    pub ttl_seconds: u64,
}

impl AppState {
    /// Load a [`ChunkManifest`] from a JSON file on disk and wrap it in
    /// [`AppState`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the bytes are not
    /// valid `ChunkManifest` JSON.
    pub async fn load_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
    ) -> Result<Self> {
        let bytes = tokio::fs::read(manifest_path)
            .await
            .with_context(|| format!("failed to read manifest from {}", manifest_path.display()))?;

        let manifest: ChunkManifest = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "failed to parse manifest JSON from {}",
                manifest_path.display()
            )
        })?;

        Ok(Self {
            manifest: Arc::new(RwLock::new(manifest)),
            cdn_base_url: Arc::new(cdn_base_url),
            ttl_seconds,
        })
    }
}
